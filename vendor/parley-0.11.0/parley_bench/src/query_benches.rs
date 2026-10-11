// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Benchmarks for queries against finished [`Layout`]s, like cursor movement.

use crate::benches::build_layout;
use crate::{ColorBrush, get_samples};
use parley::{Affinity, Alignment, AlignmentOptions, Cursor, Layout, Selection};
use std::hint::black_box;
use tango_bench::{Benchmark, benchmark_fn};

/// The width to wrap at: around 90 characters of 16px Roboto per line.
const MAX_ADVANCE: f32 = 720.;

/// A width to wrap at that gives much longer lines than [`MAX_ADVANCE`], while still leaving some
/// lines to move between.
const WIDE_MAX_ADVANCE: f32 = 10. * MAX_ADVANCE;

fn build_and_break(text: &str, max_advance: Option<f32>) -> Layout<ColorBrush> {
    let mut layout = build_layout(text, []);
    layout.break_all_lines(max_advance);
    layout.align(Alignment::Start, AlignmentOptions::default());
    layout
}

/// Byte indices spread evenly through `text`, snapped to character boundaries.
fn spread_byte_indices(text: &str, count: usize) -> Vec<usize> {
    (0..count)
        .map(|position| {
            let mut index = text.len() * position / count;
            while !text.is_char_boundary(index) {
                index -= 1;
            }
            index
        })
        .collect()
}

/// Cursors at the [spread byte indices](spread_byte_indices) of `text`.
fn spread_cursors(layout: &Layout<ColorBrush>, text: &str, count: usize) -> Vec<Cursor> {
    spread_byte_indices(text, count)
        .into_iter()
        .map(|index| Cursor::from_byte_index(layout, index, Affinity::Downstream))
        .collect()
}

/// Move each cursor of `starts` down `lines` number of lines and back up again.
///
/// Returns a value derived from what the cursors landed on, to `black_box`.
fn walk_lines(layout: &Layout<ColorBrush>, starts: &[Cursor], lines: usize) -> usize {
    let mut indices = 0;
    for start in starts {
        // Uses a [`Selection`], as that retains the cursor's horizontal position across lines.
        let mut selection = Selection::from(*start);
        for _ in 0..lines {
            selection = selection.next_line(layout, false);
        }
        for _ in 0..lines {
            selection = selection.previous_line(layout, false);
        }
        indices += selection.focus().index();
    }
    indices
}

/// Benchmark cursor building and movement.
pub fn cursor() -> Vec<Benchmark> {
    /// The number of start positions to operate from.
    const STARTS: usize = 16;
    /// The number of clusters moved over per start position.
    const CLUSTER_STEPS: usize = 16;
    /// The number of lines moved down, and back up, per start position.
    const LINE_STEPS: usize = 4;

    let mut benchmarks = Vec::new();

    for sample in get_samples()
        .iter()
        .filter(|sample| sample.modification == "8000 characters")
    {
        benchmarks.push(benchmark_fn(
            format!(
                "Query Cursor - {} {}, next visual",
                sample.name, sample.modification
            ),
            move |b| {
                let layout = build_and_break(&sample.text, Some(MAX_ADVANCE));
                let starts = spread_cursors(&layout, &sample.text, STARTS);
                b.iter(move || {
                    let mut indices = 0;
                    for &start in &starts {
                        let mut cursor = start;
                        for _ in 0..CLUSTER_STEPS {
                            cursor = cursor.next_visual(&layout);
                        }
                        indices += cursor.index();
                    }
                    black_box(indices)
                })
            },
        ));
    }

    for sample in get_samples()
        .iter()
        .filter(|sample| sample.modification == "8000 characters")
    {
        benchmarks.push(benchmark_fn(
            format!(
                "Query Cursor - {} {}, line down + up",
                sample.name, sample.modification
            ),
            move |b| {
                let layout = build_and_break(&sample.text, Some(MAX_ADVANCE));
                let starts = spread_cursors(&layout, &sample.text, STARTS);
                b.iter(move || black_box(walk_lines(&layout, &starts, LINE_STEPS)))
            },
        ));
    }

    // Two more cases, one of which tests the same as above but with a slightly bigger input, and
    // the other shouldn't be too script-sensitive: let's just test with Latin to attempt keeping
    // the number of cases somewhat in check...
    let latin = get_samples()
        .iter()
        .find(|sample| sample.name == "latin" && sample.modification == "8000 characters")
        .expect("the Latin 8000-character benchmark sample should exist");

    // With longer lines, finding the cursor's horizontal position walks more clusters per move.
    benchmarks.push(benchmark_fn(
        format!(
            "Query Cursor - {} {}, line down + up, long lines",
            latin.name, latin.modification
        ),
        move |b| {
            let layout = build_and_break(&latin.text, Some(WIDE_MAX_ADVANCE));
            let starts = spread_cursors(&layout, &latin.text, STARTS);
            b.iter(move || black_box(walk_lines(&layout, &starts, LINE_STEPS)))
        },
    ));

    benchmarks.push(benchmark_fn(
        format!(
            "Query Cursor - {} {}, from byte index",
            latin.name, latin.modification
        ),
        move |b| {
            let layout = build_and_break(&latin.text, Some(MAX_ADVANCE));
            let indices = spread_byte_indices(&latin.text, STARTS);
            b.iter(move || {
                let mut cursors = 0;
                for &index in &indices {
                    cursors +=
                        Cursor::from_byte_index(&layout, index, Affinity::Downstream).index();
                }
                black_box(cursors);
            })
        },
    ));

    benchmarks
}
