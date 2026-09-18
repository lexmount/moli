//! CSS table block-size constraints, independent of the Grid backend.
//!
//! Distribution follows Blink's table_layout_utils.cc: rowspan deficits,
//! fixed section sizes, then table -> sections -> rows. Content minima are
//! never shrunk. Percentages retain their section-specific resolution basis.

use std::{cmp::Ordering, ops::Range};

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct RowConstraint {
    pub size: f32,
    pub percent: Option<f32>,
    pub constrained: bool,
    pub has_rowspan_start: bool,
    pub ascent: Option<f32>,
    pub descent: f32,
}

#[derive(Clone, Debug)]
pub(super) struct SectionConstraint {
    pub rows: Range<usize>,
    pub fixed: Option<f32>,
    pub percent: Option<f32>,
    pub is_body: bool,
    pub size: f32,
}

#[derive(Clone, Debug)]
pub(super) struct RowspanConstraint {
    pub rows: Range<usize>,
    pub size: f32,
}

fn extent(rows: &[RowConstraint], spacing: f32) -> f32 {
    rows.iter().map(|row| row.size).sum::<f32>() + rows.len().saturating_sub(1) as f32 * spacing
}

// Blink uses LayoutUnit (1/64 CSS px), giving the rounding remainder to the
// last eligible track. Keep that deterministic behavior for fractional shares.
fn shares(extra: f32, weights: &[f32]) -> Vec<f32> {
    let sum = weights.iter().sum::<f32>();
    let mut remaining = extra;
    weights
        .iter()
        .enumerate()
        .map(|(index, weight)| {
            let value = if index + 1 == weights.len() {
                remaining
            } else {
                let ratio = if sum > 0.0 {
                    *weight / sum
                } else {
                    1.0 / weights.len() as f32
                };
                (extra * ratio * 64.0).floor() / 64.0
            };
            remaining -= value;
            value
        })
        .collect()
}

fn grow_rows(rows: &mut [RowConstraint], indices: &[usize], extra: f32, proportional: bool) {
    let weights: Vec<_> = indices
        .iter()
        .map(|&i| if proportional { rows[i].size } else { 1.0 })
        .collect();
    for (&i, delta) in indices.iter().zip(shares(extra, &weights)) {
        rows[i].size += delta;
    }
}

fn distribute_rows(
    rows: &mut [RowConstraint],
    target: f32,
    spacing: f32,
    basis: Option<f32>,
    rowspan: bool,
) {
    let mut extra = target - extent(rows, spacing);
    if extra <= 0.0 || rows.is_empty() {
        return;
    }

    let deficits: Vec<_> = rows
        .iter()
        .map(|row| {
            row.percent.zip(basis).map_or(0.0, |(percent, basis)| {
                (percent * basis - row.size).max(0.0)
            })
        })
        .collect();
    let deficit = deficits.iter().sum::<f32>();
    if deficit > 0.0 {
        let amount = extra.min(deficit);
        let eligible: Vec<_> = deficits
            .iter()
            .enumerate()
            .filter_map(|(i, &d)| (d > 0.0).then_some(i))
            .collect();
        let weights: Vec<_> = eligible.iter().map(|&i| deficits[i]).collect();
        for (&i, delta) in eligible.iter().zip(shares(amount, &weights)) {
            rows[i].size += delta;
        }
        extra -= amount;
    }
    if extra <= 0.0 {
        return;
    }

    if rowspan {
        let origins: Vec<_> = (1..rows.len())
            .filter(|&i| rows[i].has_rowspan_start)
            .collect();
        if !origins.is_empty() {
            grow_rows(rows, &origins, extra, false);
            return;
        }
    }
    let constrained =
        |row: &RowConstraint| row.constrained && (row.percent.is_none() || basis.is_some());
    let auto_nonempty: Vec<_> = (0..rows.len())
        .filter(|&i| rows[i].size > 0.0 && !constrained(&rows[i]))
        .collect();
    if !auto_nonempty.is_empty() {
        grow_rows(rows, &auto_nonempty, extra, true);
        return;
    }
    let empty: Vec<_> = (0..rows.len()).filter(|&i| rows[i].size == 0.0).collect();
    if !empty.is_empty() {
        if rowspan && empty.len() == rows.len() {
            rows[*empty.last().unwrap()].size += extra;
            return;
        }
        if !rowspan {
            let auto_empty: Vec<_> = empty
                .iter()
                .copied()
                .filter(|&i| !constrained(&rows[i]))
                .collect();
            grow_rows(
                rows,
                if auto_empty.is_empty() {
                    &empty
                } else {
                    &auto_empty
                },
                extra,
                false,
            );
            return;
        }
    }
    let nonempty: Vec<_> = (0..rows.len()).filter(|&i| rows[i].size > 0.0).collect();
    grow_rows(rows, &nonempty, extra, true);
}

/// Establish the content minimum before resolving the table's own height.
pub(super) fn resolve_minimums(
    rows: &mut [RowConstraint],
    sections: &mut [SectionConstraint],
    spans: &mut [RowspanConstraint],
    spacing: f32,
) {
    for row in rows.iter_mut() {
        row.size = row.size.max(row.ascent.unwrap_or(0.0) + row.descent);
    }
    for section in sections.iter() {
        let mut total = 0.0_f32;
        for row in &mut rows[section.rows.clone()] {
            if let Some(percent) = &mut row.percent {
                *percent = percent.min((1.0 - total).max(0.0));
                total += *percent;
            }
        }
    }
    spans.sort_by(|a, b| {
        if a.rows == b.rows {
            return b.size.total_cmp(&a.size);
        }
        if a.rows.start >= b.rows.start && a.rows.end <= b.rows.end {
            return Ordering::Less;
        }
        if b.rows.start >= a.rows.start && b.rows.end <= a.rows.end {
            return Ordering::Greater;
        }
        a.rows.start.cmp(&b.rows.start)
    });
    for span in spans {
        distribute_rows(&mut rows[span.rows.clone()], span.size, spacing, None, true);
    }
    for section in sections {
        let group_rows = &mut rows[section.rows.clone()];
        let minimum = extent(group_rows, spacing);
        if let Some(fixed) = section.fixed.filter(|&fixed| fixed > minimum) {
            distribute_rows(group_rows, fixed, spacing, Some(fixed), false);
        }
        section.size = extent(group_rows, spacing).max(section.fixed.unwrap_or(0.0));
    }
}

/// `target` excludes outer decorations and outer cell spacing, but includes
/// the gaps between nonempty sections. Empty sections have no grid tracks.
pub(super) fn distribute_table(
    rows: &mut [RowConstraint],
    sections: &mut [SectionConstraint],
    target: f32,
    spacing: f32,
) {
    if sections.is_empty() {
        return;
    }
    let target = (target - sections.len().saturating_sub(1) as f32 * spacing).max(0.0);
    let mut extra = target - sections.iter().map(|s| s.size).sum::<f32>();
    if extra <= 0.0 {
        return;
    }
    let original: Vec<_> = sections.iter().map(|s| s.size).collect();
    let deficits: Vec<_> = sections
        .iter()
        .map(|s| s.percent.map_or(0.0, |p| (p * target - s.size).max(0.0)))
        .collect();
    let deficit = deficits.iter().sum::<f32>();
    if deficit > 0.0 {
        let amount = extra.min(deficit);
        let eligible: Vec<_> = (0..sections.len()).filter(|&i| deficits[i] > 0.0).collect();
        let weights: Vec<_> = eligible.iter().map(|&i| deficits[i]).collect();
        for (&i, delta) in eligible.iter().zip(shares(amount, &weights)) {
            sections[i].size += delta;
        }
        extra -= amount;
    }
    if extra > 0.0 {
        let has_body = sections.iter().any(|s| s.is_body);
        let priority = |s: &SectionConstraint| {
            if s.percent.is_some() {
                2
            } else if s.fixed.is_some() {
                1
            } else {
                0
            }
        };
        let rank = sections
            .iter()
            .filter(|s| !has_body || s.is_body)
            .map(priority)
            .min()
            .unwrap();
        let eligible: Vec<_> = (0..sections.len())
            .filter(|&i| (!has_body || sections[i].is_body) && priority(&sections[i]) == rank)
            .collect();
        let weights: Vec<_> = eligible.iter().map(|&i| sections[i].size).collect();
        for (&i, delta) in eligible.iter().zip(shares(extra, &weights)) {
            sections[i].size += delta;
        }
    }
    for (section, original) in sections.iter().zip(original) {
        if section.size > original {
            distribute_rows(
                &mut rows[section.rows.clone()],
                section.size,
                spacing,
                Some(section.size),
                false,
            );
        }
    }
}
