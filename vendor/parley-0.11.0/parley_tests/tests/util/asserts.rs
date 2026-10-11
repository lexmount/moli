// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Various helper functions to assert truths during testing.

use parley::{Brush, Layout};

/// Assert that the two provided `Layout`s are equal in terms of alignment metrics.
pub(crate) fn assert_eq_layout_alignments<B: Brush>(a: &Layout<B>, b: &Layout<B>, case: &str) {
    // Compare line counts
    let a_line_count = a.len();
    let b_line_count = b.len();
    assert_eq!(
        a_line_count, b_line_count,
        "line count mismatch with {case}"
    );

    // Compare line offsets
    for (i, (line_a, line_b)) in a.lines().zip(b.lines()).enumerate() {
        assert_eq!(
            line_a.metrics().offset,
            line_b.metrics().offset,
            "line {i} offset mismatch with {case}"
        );
    }

    // Collect all cluster advances from both layouts
    // We need to collect runs first due to lifetime constraints
    let a_advances: Vec<f32> = collect_cluster_advances(a);
    let b_advances: Vec<f32> = collect_cluster_advances(b);

    assert_eq!(
        a_advances.len(),
        b_advances.len(),
        "cluster count mismatch with {case}"
    );

    for (i, (adv_a, adv_b)) in a_advances.iter().zip(b_advances.iter()).enumerate() {
        assert_eq!(adv_a, adv_b, "cluster {i} advance mismatch with {case}");
    }
}

/// Collect all cluster advances from a layout.
fn collect_cluster_advances<B: Brush>(layout: &Layout<B>) -> Vec<f32> {
    let mut advances = Vec::new();
    for line in layout.lines() {
        for run in line.runs() {
            for cluster in run.clusters() {
                advances.push(cluster.advance());
            }
        }
    }
    advances
}
