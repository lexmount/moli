use taffy::{TrackSizingFunction, style_helpers};

/// A width constraint collected by the CSS table formatting context.
///
/// This remains independent of Grid track sizing. In fixed table layout the
/// constraints are synchronized against the table's assignable inline size
/// first, and only the resulting used lengths are handed to the Grid backend.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct TableColumnConstraint {
    /// Intrinsic floor accumulated from cells and column boxes.
    ///
    /// An explicit column starts at `Some(0)`; `None` means an implicit
    /// column has not received a cell constraint yet. Blink preserves this
    /// distinction because colspan min/max contributions are filled
    /// independently.
    pub(super) min_inline_size: Option<f32>,
    /// Maximum/fixed measure. `is_constrained` distinguishes a declared width
    /// from an intrinsic maximum once automatic table sizing is implemented.
    pub(super) max_inline_size: Option<f32>,
    pub(super) percent: Option<f32>,
    pub(super) percent_border_padding: f32,
    pub(super) is_constrained: bool,
}

/// Inline-size information contributed by a table cell.
///
/// Keeping cell constraints separate from column constraints is important for
/// wide cells: a `colspan` cell contributes one measure over a range and must
/// not masquerade as a width authored on any individual column.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct TableCellInlineConstraint {
    pub(super) min_inline_size: f32,
    pub(super) max_inline_size: f32,
    pub(super) percent: Option<f32>,
    pub(super) percent_border_padding: f32,
    pub(super) is_constrained: bool,
}

impl TableCellInlineConstraint {
    pub(super) const fn auto() -> Self {
        Self {
            min_inline_size: 0.0,
            max_inline_size: 0.0,
            percent: None,
            percent_border_padding: 0.0,
            is_constrained: false,
        }
    }

    pub(super) fn length(value: f32) -> Self {
        Self {
            max_inline_size: value.max(0.0),
            is_constrained: true,
            ..Self::auto()
        }
    }

    pub(super) fn percent(ratio: f32, border_padding: f32) -> Self {
        Self {
            max_inline_size: border_padding.max(0.0),
            percent: Some(ratio.max(0.0)),
            percent_border_padding: border_padding.max(0.0),
            ..Self::auto()
        }
    }
}

/// A cell constraint that covers more than one column.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct TableCellSpanConstraint {
    pub(super) start_column: usize,
    pub(super) span: usize,
    pub(super) cell: TableCellInlineConstraint,
}

impl TableColumnConstraint {
    pub(super) const fn auto() -> Self {
        Self {
            min_inline_size: None,
            max_inline_size: None,
            percent: None,
            percent_border_padding: 0.0,
            is_constrained: false,
        }
    }

    pub(super) const fn explicit_auto() -> Self {
        Self {
            min_inline_size: Some(0.0),
            ..Self::auto()
        }
    }

    pub(super) fn length(value: f32) -> Self {
        Self {
            min_inline_size: Some(0.0),
            max_inline_size: Some(value.max(0.0)),
            is_constrained: true,
            ..Self::auto()
        }
    }

    pub(super) fn percent(ratio: f32, border_padding: f32) -> Self {
        Self {
            min_inline_size: Some(0.0),
            percent: Some(ratio.max(0.0)),
            percent_border_padding: border_padding.max(0.0),
            ..Self::auto()
        }
    }

    /// Apply a single-column first-row cell while preserving the precedence of
    /// an authored `<col>`/`<colgroup>` measure.
    pub(super) fn encompass_first_row_cell(&mut self, cell: TableCellInlineConstraint) {
        if self.is_constrained {
            return;
        }

        self.min_inline_size = Some(
            self.min_inline_size
                .unwrap_or(cell.min_inline_size)
                .max(cell.min_inline_size),
        );
        self.max_inline_size = Some(
            self.max_inline_size
                .unwrap_or(0.0)
                .max(cell.max_inline_size),
        );
        if cell.percent > self.percent {
            self.percent = cell.percent;
            self.percent_border_padding = cell.percent_border_padding;
        }
        self.is_constrained |= cell.is_constrained;
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
                .unwrap_or(0.0)
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
        let min_inline_size = self.min_inline_size.unwrap_or(0.0);
        if let Some(fixed) = self.fixed_inline_size() {
            min_inline_size.max(fixed)
        } else {
            min_inline_size.max(self.percent_border_padding)
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

/// Project first-row wide-cell constraints onto fixed-layout columns.
///
/// This mirrors Blink's `DistributeColspanCellToColumnsFixed`: shorter spans
/// are applied first, inner border spacing is excluded from the cell measure,
/// existing column measures retain priority, and percentage constraints are
/// copied only to unconstrained columns. The final fixed-column allocator then
/// operates solely on column constraints.
pub(super) fn distribute_fixed_cell_spans(
    column_constraints: &mut [TableColumnConstraint],
    cell_spans: &mut [TableCellSpanConstraint],
    inline_border_spacing: f32,
) {
    cell_spans.sort_by_key(|constraint| (constraint.span, constraint.start_column));

    for constraint in cell_spans {
        let Some(column_span) = column_constraints.get_mut(constraint.start_column..) else {
            continue;
        };
        let effective_span = constraint.span.min(column_span.len());
        if effective_span == 0 {
            continue;
        }
        let column_span = &mut column_span[..effective_span];
        let inner_spacing =
            inline_border_spacing.max(0.0) * effective_span.saturating_sub(1) as f32;
        let min_inline_size = if constraint.cell.is_constrained {
            (constraint.cell.min_inline_size - inner_spacing).max(0.0)
        } else {
            0.0
        };
        let max_inline_size = (constraint.cell.max_inline_size - inner_spacing).max(0.0);
        let min_share = min_inline_size / effective_span as f32;
        let max_share = max_inline_size / effective_span as f32;
        let percent_share = constraint
            .cell
            .percent
            .map(|percent| percent / effective_span as f32);

        for (index, column) in column_span.iter_mut().enumerate() {
            let is_last = index + 1 == effective_span;
            let distributed_min = if is_last {
                min_inline_size - min_share * index as f32
            } else {
                min_share
            };
            let distributed_max = if is_last {
                max_inline_size - max_share * index as f32
            } else {
                max_share
            };
            if column.min_inline_size.is_none() {
                column.min_inline_size = Some(distributed_min);
                column.is_constrained |= constraint.cell.is_constrained;
            }
            if column.max_inline_size.is_none() {
                column.max_inline_size =
                    Some(distributed_max.max(column.min_inline_size.unwrap_or(0.0)));
                column.is_constrained |= constraint.cell.is_constrained;
            }
            if column.percent.is_none() && !column.is_constrained {
                column.percent = percent_share;
                // A wide percentage cell contributes its percentage, but its
                // border/padding belongs to the spanning cell rather than any
                // one of the columns.
                column.percent_border_padding = 0.0;
            }
        }
    }
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
    fn fixed_cell_spans_apply_shorter_ranges_before_wider_ranges() {
        let mut columns = [TableColumnConstraint::auto(); 3];
        let mut spans = [
            TableCellSpanConstraint {
                start_column: 0,
                span: 3,
                cell: TableCellInlineConstraint::length(150.0),
            },
            TableCellSpanConstraint {
                start_column: 0,
                span: 2,
                cell: TableCellInlineConstraint::length(80.0),
            },
        ];

        distribute_fixed_cell_spans(&mut columns, &mut spans, 0.0);

        assert_eq!(columns[0].max_inline_size, Some(40.0));
        assert_eq!(columns[1].max_inline_size, Some(40.0));
        assert_eq!(columns[2].max_inline_size, Some(50.0));
    }

    #[test]
    fn fixed_cell_spans_preserve_columns_and_exclude_internal_spacing() {
        let mut columns = [
            TableColumnConstraint::length(80.0),
            TableColumnConstraint::auto(),
            TableColumnConstraint::auto(),
        ];
        let mut spans = [TableCellSpanConstraint {
            start_column: 0,
            span: 2,
            cell: TableCellInlineConstraint::length(210.0),
        }];

        distribute_fixed_cell_spans(&mut columns, &mut spans, 10.0);

        assert_eq!(columns[0].max_inline_size, Some(80.0));
        assert_eq!(columns[1].max_inline_size, Some(100.0));
        assert_sizes(
            &distribute_fixed_columns(300.0, &columns),
            &[80.0, 100.0, 120.0],
        );
    }

    #[test]
    fn fixed_cell_spans_divide_percent_without_cell_border_padding() {
        let mut columns = [TableColumnConstraint::auto(); 3];
        let mut spans = [TableCellSpanConstraint {
            start_column: 0,
            span: 2,
            cell: TableCellInlineConstraint::percent(0.4, 20.0),
        }];

        distribute_fixed_cell_spans(&mut columns, &mut spans, 0.0);

        assert_eq!(columns[0].percent, Some(0.2));
        assert_eq!(columns[1].percent, Some(0.2));
        assert_eq!(columns[0].percent_border_padding, 0.0);
        assert_eq!(columns[1].percent_border_padding, 0.0);
        assert_sizes(
            &distribute_fixed_columns(500.0, &columns),
            &[100.0, 100.0, 300.0],
        );
    }

    #[test]
    fn fixed_cell_spans_fill_missing_min_and_max_constraints_independently() {
        let mut columns = [
            TableColumnConstraint::explicit_auto(),
            TableColumnConstraint::percent(0.5, 0.0),
            TableColumnConstraint {
                max_inline_size: Some(70.0),
                ..TableColumnConstraint::auto()
            },
            TableColumnConstraint::auto(),
        ];
        let mut spans = [TableCellSpanConstraint {
            start_column: 0,
            span: 4,
            cell: TableCellInlineConstraint {
                min_inline_size: 80.0,
                max_inline_size: 160.0,
                percent: None,
                percent_border_padding: 0.0,
                is_constrained: true,
            },
        }];

        distribute_fixed_cell_spans(&mut columns, &mut spans, 0.0);

        assert_eq!(columns[0].min_inline_size, Some(0.0));
        assert_eq!(columns[1].min_inline_size, Some(0.0));
        assert_eq!(columns[2].min_inline_size, Some(20.0));
        assert_eq!(columns[3].min_inline_size, Some(20.0));
        assert_eq!(columns[0].max_inline_size, Some(40.0));
        assert_eq!(columns[1].max_inline_size, Some(40.0));
        assert_eq!(columns[2].max_inline_size, Some(70.0));
        assert_eq!(columns[3].max_inline_size, Some(40.0));
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
