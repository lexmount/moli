// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Line breaking tests.
//!
//! These tests validate `break_next_with_length`, which breaks lines by character count
//! rather than advance width.

use crate::test_name;
use crate::util::TestEnv;
use parley::VerticalAlign;
use parley::style::FontFamily;
use parley::{Alignment, AlignmentOptions, InlineBox, InlineBoxKind, StyleProperty, YieldData};

#[test]
fn break_by_length_basic() {
    let mut env = TestEnv::new(test_name!(), None);

    // 6 characters "ABCDEF", break every 2 clusters -> 3 lines
    let text = "ABCDEF";
    let builder = env.ranged_builder(text);
    let mut layout = builder.build(text);

    let mut breaker = layout.break_lines();
    breaker.break_next_with_length(2);
    breaker.break_next_with_length(2);
    breaker.break_next_with_length(2);
    breaker.finish();

    assert_eq!(layout.len(), 3, "Expected 3 lines");
    let cluster_ranges: Vec<_> = layout
        .lines()
        .flat_map(|line| line.runs().map(|run| run.cluster_range()))
        .collect();
    assert_eq!(cluster_ranges, vec![0..2, 2..4, 4..6]);
    env.check_layout_snapshot(&layout);
}

#[test]
fn break_by_length_varying_lengths() {
    let mut env = TestEnv::new(test_name!(), None);

    // 10 characters "ABCDEFGHIJ", break at 3, 4, 3 clusters
    let text = "ABCDEFGHIJ";
    let builder = env.ranged_builder(text);
    let mut layout = builder.build(text);

    let mut breaker = layout.break_lines();
    breaker.break_next_with_length(3); // ABC
    breaker.break_next_with_length(4); // DEFG
    breaker.break_next_with_length(3); // HIJ
    breaker.finish();

    assert_eq!(layout.len(), 3, "Expected 3 lines");
    env.check_layout_snapshot(&layout);
}

#[test]
fn break_by_length_with_spaces() {
    let mut env = TestEnv::new(test_name!(), None);

    // "AB CD" = 5 clusters (space counts)
    let text = "AB CD";
    let builder = env.ranged_builder(text);
    let mut layout = builder.build(text);

    let mut breaker = layout.break_lines();
    breaker.break_next_with_length(3); // "AB " (including space)
    breaker.break_next_with_length(2); // "CD"
    breaker.finish();

    assert_eq!(layout.len(), 2, "Expected 2 lines");
    env.check_layout_snapshot(&layout);
}

#[test]
fn break_by_length_with_newline() {
    let mut env = TestEnv::new(test_name!(), None);

    // "AB\nCD" = 5 clusters, newline counts as 1 and does NOT cause automatic break
    let text = "AB\nCD";
    let builder = env.ranged_builder(text);
    let mut layout = builder.build(text);

    // Break at 4 clusters: should get "AB\nC" on first line, "D" on second
    let mut breaker = layout.break_lines();
    breaker.break_next_with_length(4);
    breaker.break_next_with_length(10); // remaining
    breaker.finish();

    assert_eq!(layout.len(), 2, "Expected 2 lines");
    env.check_layout_snapshot(&layout);
}

#[test]
fn break_by_length_with_inline_box() {
    let mut env = TestEnv::new(test_name!(), None);

    // "A[box]BC" where box counts as 1 cluster
    let text = "ABC";
    let mut builder = env.ranged_builder(text);
    builder.push_inline_box(InlineBox {
        id: 0,
        kind: InlineBoxKind::InFlow,
        index: 1, // After 'A'
        width: 10.0,
        height: 10.0,
        baseline: None,
        vertical_align: VerticalAlign::BASELINE,
    });
    let mut layout = builder.build(text);

    // Break at 2 clusters: "A[box]" on first line, "BC" on second
    let mut breaker = layout.break_lines();
    breaker.break_next_with_length(2);
    breaker.break_next_with_length(10);
    breaker.finish();

    assert_eq!(layout.len(), 2, "Expected 2 lines");
    env.check_layout_snapshot(&layout);
}

#[test]
fn break_by_length_multiple_inline_boxes() {
    let mut env = TestEnv::new(test_name!(), None);

    // "[box][box][box]ABC" - 3 boxes + 3 chars = 6 clusters
    let text = "ABC";
    let mut builder = env.ranged_builder(text);
    for id in 0..3 {
        builder.push_inline_box(InlineBox {
            id,
            kind: InlineBoxKind::InFlow,
            index: 0, // All at the start
            width: 10.0,
            height: 10.0,
            baseline: None,
            vertical_align: VerticalAlign::BASELINE,
        });
    }
    let mut layout = builder.build(text);

    // Break at 2 clusters each -> 3 lines
    let mut breaker = layout.break_lines();
    breaker.break_next_with_length(2);
    breaker.break_next_with_length(2);
    breaker.break_next_with_length(2);
    breaker.finish();

    assert_eq!(layout.len(), 3, "Expected 3 lines");
    env.check_layout_snapshot(&layout);
}

/// This test verifies that a ligature is never split across lines.
///
/// A ligature is a single unbreakable atom: splitting it would require re-shaping, which parley
/// does not currently support. When the character limit is reached inside the "fi" ligature of
/// "abfi", the whole ligature is placed on the line before breaking.
#[test]
fn break_by_length_with_ligature() {
    let mut env = TestEnv::new(test_name!(), None);

    // "abfi" has ligature "fi", which forms a single unbreakable atom
    let text = "abfi";
    let builder = env.ranged_builder(text);
    let mut layout = builder.build(text);

    // Attempt to break at 3 characters: "abf" on the first line", "i" on the second
    let mut breaker = layout.break_lines();
    breaker.break_next_with_length(3);
    breaker.break_next_with_length(10);
    breaker.finish();

    assert_eq!(layout.len(), 1, "Expected a single line");

    // The glyphs of the ligature stay together: "a", "b" and the "fi" all render on the first line
    let line = layout.get(0).expect("Expected line 1 to exist");
    let glyph_count: usize = line
        .runs()
        .flat_map(|run| run.clusters())
        .map(|cluster| cluster.glyphs().count())
        .sum();
    assert!(
        glyph_count >= 3,
        "expected glyphs for 'a', 'b' and the 'fi' ligature"
    );

    env.check_layout_snapshot(&layout);
}

#[test]
fn break_by_length_exact_fit() {
    let mut env = TestEnv::new(test_name!(), None);

    // "ABCD" = 4 clusters, break at exactly 4
    let text = "ABCD";
    let builder = env.ranged_builder(text);
    let mut layout = builder.build(text);

    let mut breaker = layout.break_lines();
    breaker.break_next_with_length(4);
    breaker.finish();

    assert_eq!(layout.len(), 1, "Expected 1 line");
    env.check_layout_snapshot(&layout);
}

#[test]
fn break_by_length_single_cluster_lines() {
    let mut env = TestEnv::new(test_name!(), None);

    // "ABC" = 3 clusters, break every 1 cluster -> 3 lines
    let text = "ABC";
    let builder = env.ranged_builder(text);
    let mut layout = builder.build(text);

    let mut breaker = layout.break_lines();
    breaker.break_next_with_length(1);
    breaker.break_next_with_length(1);
    breaker.break_next_with_length(1);
    breaker.finish();

    assert_eq!(layout.len(), 3, "Expected 3 lines");
    env.check_layout_snapshot(&layout);
}

#[test]
fn break_by_length_with_emoji() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "✅👀🎉🤠✅👀";
    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::FontFamily(FontFamily::named(
        "Noto Color Emoji",
    )));
    let mut layout = builder.build(text);

    // Break at 1, 3, 2 clusters -> expect 3 lines
    let mut breaker = layout.break_lines();
    breaker.break_next_with_length(1);
    breaker.break_next_with_length(3);
    breaker.break_next_with_length(2);
    breaker.finish();

    assert_eq!(layout.len(), 3, "Expected 3 lines");
    env.check_layout_snapshot(&layout);
}

#[test]
fn break_by_length_with_emoji_only() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "✅👀🎉🤠";
    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::FontFamily(FontFamily::named(
        "Noto Color Emoji",
    )));
    let mut layout = builder.build(text);

    // Break at 2 clusters each -> expect 2 lines
    let mut breaker = layout.break_lines();
    breaker.break_next_with_length(2);
    breaker.break_next_with_length(2);
    breaker.finish();

    assert_eq!(layout.len(), 2, "Expected 2 lines");
    env.check_layout_snapshot(&layout);
}

#[test]
fn break_by_length_with_multi_codepoint_emoji() {
    let mut env = TestEnv::new(test_name!(), None);

    // Family emoji (ZWJ sequence): 👨‍👩‍👧‍👦 = 7 codepoints
    // Flag emoji: 🇺🇸 = 2 codepoints
    // Simple emoji: ✅ = 1 codepoint (in the bundled subset)
    // Total = 10 characters, forming 3 grapheme clusters
    let text = "👨‍👩‍👧‍👦🇺🇸✅";
    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::FontFamily(FontFamily::named(
        "Noto Color Emoji",
    )));
    let mut layout = builder.build(text);

    // Break at visual emoji boundaries: 7 (family), 2 (flag), 1 (simple) characters
    let mut breaker = layout.break_lines();
    breaker.break_next_with_length(7); // 👨‍👩‍👧‍👦
    breaker.break_next_with_length(2); // 🇺🇸
    breaker.break_next_with_length(1); // ✅
    breaker.finish();

    assert_eq!(layout.len(), 3, "Expected 3 lines");

    // Each emoji is a single grapheme cluster, i.e., each line holds exactly one cluster
    let clusters_per_line: Vec<usize> = layout
        .lines()
        .map(|line| line.runs().map(|run| run.clusters().count()).sum())
        .collect();
    assert_eq!(clusters_per_line, vec![1, 1, 1]);
}

/// This test verifies that `break_next_with_length` produces the same layout metrics as
/// `break_all_lines` when breaking at the same cluster positions.
///
/// The test applies letter spacing to text with newlines and compares:
/// 1. A layout broken with `break_all_lines(None)` (breaks on newlines only)
/// 2. A layout broken with `break_next_with_length` using cluster counts from (1)
#[test]
fn break_by_length_matches_max_advance_with_letter_spacing() {
    let mut env = TestEnv::new(test_name!(), None);

    // Text with newlines and multiple spaces as the user specified
    let text = "01 2  \n\n  34\n 5";

    // Create first layout with letter spacing, broken by max_advance (respects newlines)
    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::LetterSpacing(2.0));
    let mut layout_max_advance = builder.build(text);
    layout_max_advance.break_all_lines(None);
    layout_max_advance.align(Alignment::Start, AlignmentOptions::default());

    // Count clusters per line from the max_advance layout using public API
    let cluster_counts: Vec<u32> = layout_max_advance
        .lines()
        .map(|line| line.runs().map(|run| run.len() as u32).sum())
        .collect();

    // Create second layout with the same letter spacing, broken by length
    let mut builder2 = env.ranged_builder(text);
    builder2.push_default(StyleProperty::LetterSpacing(2.0));
    let mut layout_by_length = builder2.build(text);

    let mut breaker = layout_by_length.break_lines();
    for count in &cluster_counts {
        breaker.break_next_with_length(*count);
    }
    breaker.finish();
    layout_by_length.align(Alignment::Start, AlignmentOptions::default());

    // Compare the layouts - they should have the same line count
    assert_eq!(
        layout_max_advance.len(),
        layout_by_length.len(),
        "Line count mismatch: max_advance={}, by_length={}",
        layout_max_advance.len(),
        layout_by_length.len()
    );

    // Compare line-by-line metrics
    for (i, (line_a, line_b)) in layout_max_advance
        .lines()
        .zip(layout_by_length.lines())
        .enumerate()
    {
        assert_eq!(
            line_a.metrics().offset,
            line_b.metrics().offset,
            "Line {i} offset mismatch: max_advance={}, by_length={}",
            line_a.metrics().offset,
            line_b.metrics().offset
        );
        assert_eq!(
            line_a.metrics().advance,
            line_b.metrics().advance,
            "Line {i} advance mismatch: max_advance={}, by_length={}",
            line_a.metrics().advance,
            line_b.metrics().advance
        );
        assert_eq!(
            line_a.text_range(),
            line_b.text_range(),
            "Line {i} text_range mismatch"
        );
    }

    // Compare run cluster ranges between the two layouts
    for (i, (run_a, run_b)) in layout_max_advance
        .lines()
        .flat_map(|line| line.runs())
        .zip(layout_by_length.lines().flat_map(|line| line.runs()))
        .enumerate()
    {
        assert_eq!(
            run_a.cluster_range(),
            run_b.cluster_range(),
            "Run {i} cluster_range mismatch"
        );
    }

    // Compare overall layout dimensions
    assert_eq!(
        layout_max_advance.width(),
        layout_by_length.width(),
        "Layout width mismatch"
    );
    assert_eq!(
        layout_max_advance.height(),
        layout_by_length.height(),
        "Layout height mismatch"
    );

    // Check both snapshots
    env.with_name("max_advance")
        .check_layout_snapshot(&layout_max_advance);
    env.with_name("by_length")
        .check_layout_snapshot(&layout_by_length);
}

/// This test verifies that the last line of justified text is start-aligned when using
/// `break_next_with_length`, matching the behavior of `break_all_lines`.
#[test]
fn break_by_length_justified_last_line_start_aligned() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "AAA BBB CCC";

    // Create layout with break_all_lines and justify
    let builder = env.ranged_builder(text);
    let mut layout_max_advance = builder.build(text);
    layout_max_advance.break_all_lines(Some(60.0)); // Force wrapping
    layout_max_advance.align(Alignment::Justify, AlignmentOptions::default());

    // Get cluster counts per line using public API
    let cluster_counts: Vec<u32> = layout_max_advance
        .lines()
        .map(|line| line.runs().map(|run| run.len() as u32).sum())
        .collect();

    // Create layout with break_next_with_length and justify
    let builder2 = env.ranged_builder(text);
    let mut layout_by_length = builder2.build(text);
    let mut breaker = layout_by_length.break_lines();
    for count in &cluster_counts {
        breaker.break_next_with_length(*count);
    }
    breaker.finish();
    layout_by_length.align(Alignment::Justify, AlignmentOptions::default());

    // Verify both layouts have the same number of lines
    assert_eq!(layout_max_advance.len(), layout_by_length.len());

    // Verify the break reasons match (important for justification behavior)
    for (i, (line_a, line_b)) in layout_max_advance
        .lines()
        .zip(layout_by_length.lines())
        .enumerate()
    {
        assert_eq!(
            line_a.break_reason(),
            line_b.break_reason(),
            "Line {i} break_reason mismatch: max_advance={:?}, by_length={:?}",
            line_a.break_reason(),
            line_b.break_reason()
        );
    }

    // Check both snapshots
    env.with_name("max_advance")
        .check_layout_snapshot(&layout_max_advance);
    env.with_name("by_length")
        .check_layout_snapshot(&layout_by_length);
}

#[test]
fn raising_line_max_height_clears_max_height_exceeded() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "Hello world";
    let builder = env.ranged_builder(text);
    let mut layout = builder.build(text);

    let mut breaker = layout.break_lines();
    breaker.state_mut().set_line_max_height(1.0);

    let mut max_height_exceeded_count = 0;
    let mut iterations = 0;
    while let Some(data) = breaker.break_next() {
        iterations += 1;
        assert!(iterations < 100, "Line breaking did not terminate");
        if let YieldData::MaxHeightExceeded(_) = data {
            max_height_exceeded_count += 1;
            breaker.state_mut().set_line_max_height(f32::INFINITY);
        }
    }
    breaker.finish();

    assert_eq!(max_height_exceeded_count, 1);
}
