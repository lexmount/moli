// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Various helper functions to assert truths during testing.

use std::vec::Vec;

use crate::{Brush, data::LayoutData};

fn canonicalize_layout_data<B: Brush>(layout_data: &LayoutData<B>) -> LayoutData<B> {
    let mut normalized = layout_data.clone();
    let mut canonical_styles = Vec::with_capacity(normalized.styles.len());
    let mut canonical_metrics = Vec::with_capacity(normalized.styles.len());
    let mut remap = Vec::with_capacity(normalized.styles.len());

    // The style tree (`parent`) and everything derived from it (the parent-relative
    // `baseline_offset` and `aligned_subtree_root` of the style metrics) is intentionally not
    // part of the comparison: the tree builder records span nesting that the flat builders
    // cannot express, so only the visual style properties are compared.
    for (style, metrics) in normalized.styles.iter().zip(&normalized.style_metrics) {
        let mut style = style.clone();
        style.parent = 0;
        let mut metrics = *metrics;
        metrics.baseline_offset = 0.;
        metrics.aligned_subtree_root = 0;
        metrics.parent = 0;
        if let Some(index) = canonical_styles
            .iter()
            .position(|existing| *existing == style)
        {
            remap.push(index as u16);
        } else {
            let index = canonical_styles.len() as u16;
            canonical_styles.push(style);
            canonical_metrics.push(metrics);
            remap.push(index);
        }
    }

    normalized.shaped_text.remap_styles(&remap);
    normalized.styles = canonical_styles;
    normalized.style_metrics = canonical_metrics;
    normalized
}

/// Assert that the two provided `LayoutData` are equal.
pub(crate) fn assert_eq_layout_data<B: Brush>(a: &LayoutData<B>, b: &LayoutData<B>, case: &str) {
    let a = canonicalize_layout_data(a);
    let b = canonicalize_layout_data(b);

    assert_eq!(a.scale, b.scale, "{case} scale mismatch");
    assert_eq!(a.quantize, b.quantize, "{case} quantize mismatch");
    assert_eq!(a.base_level, b.base_level, "{case} base_level mismatch");
    assert_eq!(a.text_len, b.text_len, "{case} text_len mismatch");
    assert_eq!(a.width, b.width, "{case} width mismatch");
    assert_eq!(a.full_width, b.full_width, "{case} full_width mismatch");
    assert_eq!(a.height, b.height, "{case} height mismatch");
    assert_eq!(
        a.shaped_text.fonts(),
        b.shaped_text.fonts(),
        "{case} fonts mismatch"
    );
    assert_eq!(
        a.shaped_text.normalized_coords(),
        b.shaped_text.normalized_coords(),
        "{case} coords mismatch"
    );

    // Input (/ output of style resolution)
    assert_eq!(a.styles, b.styles, "{case} styles mismatch");
    assert_eq!(
        a.style_metrics, b.style_metrics,
        "{case} style_metrics mismatch"
    );
    assert_eq!(
        a.inline_boxes, b.inline_boxes,
        "{case} inline_boxes mismatch"
    );

    // Output of shaping
    assert_eq!(a.runs, b.runs, "{case} runs mismatch");
    assert_eq!(a.items, b.items, "{case} items mismatch");
    assert_eq!(
        a.shaped_text.characters(),
        b.shaped_text.characters(),
        "{case} characters mismatch"
    );
    assert_eq!(
        a.shaped_text.shaped_clusters(),
        b.shaped_text.shaped_clusters(),
        "{case} clusters mismatch"
    );
    assert_eq!(
        a.shaped_text.glyphs(),
        b.shaped_text.glyphs(),
        "{case} glyphs mismatch"
    );

    // Output of line breaking
    assert_eq!(a.lines, b.lines, "{case} lines mismatch");
    assert_eq!(a.line_items, b.line_items, "{case} line_items mismatch");

    // Output of alignment
    assert_eq!(
        a.layout_max_advance, b.layout_max_advance,
        "{case} alignment_width mismatch"
    );

    // Also compare the whole struct in case any fields have been added that aren't
    // part of this test yet. If this triggers, add the missing assert to the above set.
    assert_eq!(a, b, "{case} LayoutData mismatch");
}
