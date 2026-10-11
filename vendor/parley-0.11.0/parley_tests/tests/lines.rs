// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Line layout tests.
//!
//! These tests validate the vertical positioning of lines, including metrics like
//! ascent, descent, and line box height.

use crate::test_name;
use crate::util::{ColorBrush, TestEnv};
use parley::VerticalAlign;
use parley::{
    Affinity, Alignment, AlignmentOptions, BoundingBox, Brush, Cursor, InlineBox, InlineBoxKind,
    Layout, Line, LineHeight, Selection, StyleProperty,
};
use peniko::kurbo::Size;

const TEXT: &str = "Some text here. Let's make\n\
        it a bit longer so that\n \
        we have more lines.\n\
        And also some latin text for\n\
        this spot right here.\n\
        This is underline and\n\
        strikethrough text and some\n\
        extra more text\n\
        and some extra more text\nand some extra more text\n\
        and some extra more text\nand some extra more text\n\
        and some extra more text\nand some extra more text";

const LINE_COUNT: f32 = 14.;

/// Returns the integral max advance that these tests are designed around.
fn max_advance(font_size: f32) -> f32 {
    (200. / 16. * font_size).ceil()
}

/// Returns the precise ascent and descent of Roboto.
fn roboto_ascent_descent(font_size: f32) -> (f32, f32) {
    // We calculate it based on these 16px values
    let ascent = 14.84375;
    let descent = 3.90625;
    (ascent / 16. * font_size, descent / 16. * font_size)
}

/// Line-wide text metrics derived from a line's runs.
///
/// Ascent, descent, and leading are per-run metrics; the line-wide values these tests
/// reason about are the maxima over the runs on the line (ignoring inline boxes), with
/// the leading defined as the text's intrinsic line height minus ascent and descent.
struct TextMetrics {
    ascent: f32,
    descent: f32,
    leading: f32,
}

/// Returns the line-wide text metrics for a line.
fn line_text_metrics<B: Brush>(line: &Line<'_, B>) -> TextMetrics {
    // Calculating the running max of these metrics over a line doesn't really have a typographic
    // meaning, but the tests still depend on them (we used to calculate and store these in
    // `LineMetrics`).
    let mut ascent = 0_f32;
    let mut descent = 0_f32;
    for run in line.runs() {
        let metrics = run.font_metrics();
        ascent = ascent.max(metrics.ascent);
        descent = descent.max(metrics.descent);
    }
    TextMetrics {
        ascent,
        descent,
        leading: line.metrics().line_height - (ascent + descent),
    }
}

/// Returns the expected outputs for ascent, descent, and line box height.
fn ascent_descent_box_height(font_size: f32, line_height_px: f32) -> (f32, f32, f32) {
    let (ascent, descent) = roboto_ascent_descent(font_size);
    // The line height is the one the user specified, but we quantize it to whole pixels.
    (ascent, descent, line_height_px.round())
}

/// Returns selection geometry such that every line is covered.
fn get_selections<B: Brush>(layout: &Layout<B>) -> Vec<(BoundingBox, usize)> {
    let selection_parts = [
        ("Some text here.", 4),
        ("a bit longer", 8),
        ("we have more", 5),
        ("also some latin", 9),
        ("this spot", 7),
        ("is under", 6),
        ("ough text", 4),
        ("more", 4),
        ("me ex", 5),
        ("some", 4),
        ("me ex", 5),
        ("some", 4),
        ("me ex", 5),
        ("some", 4),
    ];

    let mut selections = Vec::new();
    let mut idx = 0;
    for (substr, len) in selection_parts {
        let i = idx + TEXT[idx..].find(substr).unwrap();
        let j = i + len;
        idx = j;
        let selection = Selection::new(
            Cursor::from_byte_index(layout, i, Affinity::Downstream),
            Cursor::from_byte_index(layout, j, Affinity::Downstream),
        );
        selections.extend(selection.geometry(layout));
    }
    selections
}

/// Returns the layout.
fn build_layout<A: Into<Option<f32>>>(
    env: &mut TestEnv,
    font_size: f32,
    line_height: f32,
    max_advance: A,
) -> Layout<ColorBrush> {
    let mut builder = env.ranged_builder(TEXT);
    builder.push_default(StyleProperty::FontSize(font_size));
    builder.push_default(LineHeight::Absolute(line_height));

    let underline_style = StyleProperty::Underline(true);
    let strikethrough_style = StyleProperty::Strikethrough(true);

    // Set the underline & strikethrough style
    let pos_u = TEXT.find("underline").unwrap();
    builder.push(underline_style, pos_u..pos_u + "underline".len());
    let pos_s = TEXT.find("strikethrough").unwrap();
    builder.push(strikethrough_style, pos_s..pos_s + "strikethrough".len());

    builder.push_inline_box(InlineBox {
        id: 0,
        kind: InlineBoxKind::InFlow,
        index: 40,
        width: 50.0,
        height: 5.0,
        baseline: None,
        vertical_align: VerticalAlign::BASELINE,
    });
    builder.push_inline_box(InlineBox {
        id: 1,
        kind: InlineBoxKind::InFlow,
        index: 51,
        width: 50.0,
        height: 3.0,
        baseline: None,
        vertical_align: VerticalAlign::BASELINE,
    });

    let mut layout = builder.build(TEXT);
    layout.break_all_lines(max_advance.into());
    layout
}

/// Returns the test environment, the layout, and the selections.
fn compute(
    test_name: &str,
    font_size: f32,
    line_height_px: f32,
) -> (TestEnv, Layout<ColorBrush>, Vec<(BoundingBox, usize)>) {
    // Use max advance as the target width to ensure consistency in case of early line breaks
    let width = max_advance(font_size);
    // Calculate precise total height based on requested line height,
    // then round it to match Chrome.
    let height = (LINE_COUNT * line_height_px).round();
    // Compute the layout
    let size = Size::new(width as f64, height as f64);
    let mut env = TestEnv::new(test_name, size);
    let layout = build_layout(&mut env, font_size, line_height_px, width);
    let selections = get_selections(&layout);
    (env, layout, selections)
}

/// Assert expectations that are common across tests.
fn assert_common_truths(
    layout: &Layout<ColorBrush>,
    ascent: f32,
    descent: f32,
    line_box_height: f32,
    line_height_px: f32,
) {
    let layout_height = LINE_COUNT * line_height_px;
    assert_eq!(
        layout.height(),
        layout_height,
        "expected layout height {layout_height}"
    );
    for line in layout.lines() {
        let metrics = line.metrics();
        let text_metrics = line_text_metrics(&line);
        assert_eq!(text_metrics.ascent, ascent, "expected ascent {ascent}");
        assert_eq!(text_metrics.descent, descent, "expected descent {descent}");
        assert_eq!(
            metrics.block_max_coord - metrics.block_min_coord,
            line_box_height,
            "expected line box height {line_box_height}"
        );
        assert_eq!(
            metrics.line_height, line_height_px,
            "expected line height {line_height_px}"
        );
        assert_eq!(metrics.baseline.fract(), 0., "expected integral baseline");
    }
}

/// Test the happy path of integral line height with no leading.
#[test]
fn lines_integral_line_height_zero_leading() {
    // Inputs
    let font_size = 16.0;
    let line_height_px = 19.0;

    // Expected outputs
    let (ascent, descent, line_box_height) = ascent_descent_box_height(font_size, line_height_px);

    // Compute
    let (mut env, layout, selections) = compute(test_name!(), font_size, line_height_px);

    // Confirm metrics
    assert_common_truths(&layout, ascent, descent, line_box_height, line_height_px);
    for line in layout.lines() {
        assert!(
            ascent.fract() >= 0.5,
            "expected ascent {ascent} to round up"
        );
        assert!(
            descent.fract() >= 0.5,
            "expected descent {descent} to round up"
        );
        let leading =
            line_text_metrics(&line).leading - (1. - ascent.fract()) - (1. - descent.fract());
        assert_eq!(leading, 0., "expected zero leading");
    }

    // Verify visuals
    env.render_and_check_snapshot(&layout, None, &selections);
}

/// Test integral line height that gives a negative leading of -1.
#[test]
fn lines_integral_line_height_minus_one_leading() {
    // Inputs
    let font_size = 17.0;
    let line_height_px = 19.0;

    // Expected outputs
    let (ascent, descent, line_box_height) = ascent_descent_box_height(font_size, line_height_px);

    // Compute
    let (mut env, layout, selections) = compute(test_name!(), font_size, line_height_px);

    // Confirm metrics
    assert_common_truths(&layout, ascent, descent, line_box_height, line_height_px);
    for line in layout.lines() {
        assert!(
            ascent.fract() >= 0.5,
            "expected ascent {ascent} to round up"
        );
        assert!(
            descent.fract() < 0.5,
            "expected descent {descent} to round down"
        );
        let leading = line_text_metrics(&line).leading - (1. - ascent.fract()) + descent.fract();
        assert_eq!(leading, -1., "expected -1 leading");
    }

    // Verify visuals
    env.render_and_check_snapshot(&layout, None, &selections);
}

/// Test integral line height that gives a positive leading of 1.
///
/// The line box height must be increased by the leading on the correct side of the baseline.
#[test]
fn lines_integral_line_height_plus_one_leading() {
    // Inputs
    let font_size = 15.0;
    let line_height_px = 19.0;

    // Expected outputs
    let (ascent, descent, line_box_height) = ascent_descent_box_height(font_size, line_height_px);

    // Compute
    let (mut env, layout, selections) = compute(test_name!(), font_size, line_height_px);

    // Confirm metrics
    assert_common_truths(&layout, ascent, descent, line_box_height, line_height_px);
    for line in layout.lines() {
        assert!(
            ascent.fract() >= 0.5,
            "expected ascent {ascent} to round up"
        );
        assert!(
            descent.fract() >= 0.5,
            "expected descent {descent} to round up"
        );
        let leading =
            line_text_metrics(&line).leading - (1. - ascent.fract()) - (1. - descent.fract());
        assert_eq!(leading, 1., "expected +1 leading");

        let above = line.metrics().baseline - line.metrics().block_min_coord;
        assert_eq!(
            above,
            ascent.round(),
            "expected above to be exactly rounded ascent {}",
            ascent.round()
        );
        let below = line.metrics().block_max_coord - line.metrics().baseline;
        assert_eq!(
            below,
            descent.round() + 1.,
            "expected below to be exactly rounded descent {} + 1. = {}",
            descent.round(),
            descent.round() + 1.
        );
    }

    // Verify visuals
    env.render_and_check_snapshot(&layout, None, &selections);
}

/// Test that ascent and descent are rounded separately and then summed.
///
/// With Roboto 19.0px the ascent and descent are 17.626953 and 4.638672.
/// When rounding before summing we get 23, when rounding after summing we get 22.
/// Chrome renders the selection box with a height of 23px.
///
/// Roboto 20.0px would be another example with 18.554688 and 4.8828125.
#[test]
fn lines_integral_line_height_ascent_descent_rounding() {
    // Inputs
    let font_size = 19.0;
    let line_height_px = 20.0; // Just something lower than 23.0 (ascent+descent)

    // Expected outputs
    let (ascent, descent, line_box_height) = ascent_descent_box_height(font_size, line_height_px);

    // Compute
    let (mut env, layout, selections) = compute(test_name!(), font_size, line_height_px);

    // Confirm metrics
    assert_common_truths(&layout, ascent, descent, line_box_height, line_height_px);
    for line in layout.lines() {
        let text_metrics = line_text_metrics(&line);
        let ascent_descent_round_before_sum =
            text_metrics.ascent.round() + text_metrics.descent.round();
        let ascent_descent_round_after_sum = (text_metrics.ascent + text_metrics.descent).round();
        assert_ne!(
            ascent_descent_round_before_sum, ascent_descent_round_after_sum,
            "expected ascent and descent to be such that the ordering of round and sum matters"
        );
    }

    // Verify visuals
    env.render_and_check_snapshot(&layout, None, &selections);
}

/// Test handling of line height that rounds up both individually and also as the total layout sum.
#[test]
fn lines_line_height_rounds_up() {
    // Inputs
    let font_size = 16.0;
    let line_height_px = 20.7; // Greater than ascent + descent

    // Expected outputs
    let (ascent, descent, line_box_height) = ascent_descent_box_height(font_size, line_height_px);

    // Compute
    let (mut env, layout, selections) = compute(test_name!(), font_size, line_height_px);

    // Confirm metrics
    assert_common_truths(&layout, ascent, descent, line_box_height, line_height_px);
    assert!(
        layout.height().fract() >= 0.5,
        "expected layout height to be fractional and round up"
    );
    for line in layout.lines() {
        let metrics = line.metrics();
        assert!(
            ascent + descent < metrics.line_height,
            "expected line height {} to be greater than {} (ascent {} + descent {})",
            metrics.line_height,
            ascent + descent,
            ascent,
            descent,
        );
        assert!(
            metrics.line_height.fract() >= 0.5,
            "expected line height {} to be fractional and round up",
            metrics.line_height,
        );
        assert_eq!(
            metrics.line_height.ceil(),
            line_box_height,
            "expected line box height {} to equal line height {} rounded up",
            line_box_height,
            metrics.line_height,
        );
    }

    // Verify visuals
    env.render_and_check_snapshot(&layout, None, &selections);
}

/// Test handling of line height that rounds down both individually and also as the total layout sum.
#[test]
fn lines_line_height_rounds_down() {
    // Inputs
    let font_size = 16.0;
    let line_height_px = 20.3; // Greater than ascent + descent

    // Expected outputs
    let (ascent, descent, line_box_height) = ascent_descent_box_height(font_size, line_height_px);

    // Compute
    let (mut env, layout, selections) = compute(test_name!(), font_size, line_height_px);

    // Confirm metrics
    assert_common_truths(&layout, ascent, descent, line_box_height, line_height_px);
    assert!(
        layout.height().fract() > 0. && layout.height().fract() < 0.5,
        "expected layout height to be fractional and round down"
    );
    for line in layout.lines() {
        let metrics = line.metrics();
        assert!(
            ascent + descent < metrics.line_height,
            "expected line height {} to be greater than {} (ascent {} + descent {})",
            metrics.line_height,
            ascent + descent,
            ascent,
            descent,
        );
        assert!(
            metrics.line_height.fract() > 0. && metrics.line_height.fract() < 0.5,
            "expected line height {} to be fractional and round down",
            metrics.line_height,
        );
        assert_eq!(
            metrics.line_height.floor(),
            line_box_height,
            "expected line box height {} to equal line height {} rounded down",
            line_box_height,
            metrics.line_height,
        );
    }

    // Verify visuals
    env.render_and_check_snapshot(&layout, None, &selections);
}

/// Test fractional line height with a negative leading.
fn lines_fractional_line_height_negative_leading_internal(
    test_name: &str,
    font_size: f32,
    line_height_px: f32,
) {
    // Expected outputs
    let (ascent, descent, line_box_height) = ascent_descent_box_height(font_size, line_height_px);

    // Compute
    let (mut env, layout, selections) = compute(test_name, font_size, line_height_px);

    // Confirm metrics
    assert_common_truths(&layout, ascent, descent, line_box_height, line_height_px);
    for line in layout.lines() {
        let text_metrics = line_text_metrics(&line);
        assert!(
            text_metrics.leading < 0.,
            "expected negative leading, but got {}",
            text_metrics.leading
        );
    }

    // Verify visuals
    env.render_and_check_snapshot(&layout, None, &selections);
}

/// Test fractional line height with a negative leading.
#[test]
fn lines_fractional_line_height_negative_leading() {
    // Inputs
    let font_size = 16.0;
    let line_height_px = 12.75;

    // Run the test
    lines_fractional_line_height_negative_leading_internal(test_name!(), font_size, line_height_px);
}

/// Test fractional line height with a big negative leading.
///
/// NOTE: Going even lower (than the 0.675em in this test) will start divergence from Chrome.
#[test]
fn lines_fractional_line_height_big_negative_leading() {
    // Inputs
    let font_size = 16.0;
    let line_height_px = 10.8;

    // Run the test
    lines_fractional_line_height_negative_leading_internal(test_name!(), font_size, line_height_px);
}

/// Test that an inline box interacts correctly with negative typographic leading.
///
/// An inline box should grow its line's line box so it fits. Negative leading just means glyphs
/// from other lines may overlap this box.
#[test]
fn lines_negative_leading_inline_box_grows_line_box() {
    // Inputs
    let font_size = 16.0;
    let line_height_px = 12.0;
    let box_height = 14.0;
    let text = "dig\npug\nyak";

    let mut env = TestEnv::new(test_name!(), None);
    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::FontSize(font_size));
    builder.push_default(LineHeight::Absolute(line_height_px));
    builder.push_inline_box(InlineBox {
        id: 0,
        kind: InlineBoxKind::InFlow,
        index: 5,
        width: 12.0,
        height: box_height,
        baseline: None,
        vertical_align: VerticalAlign::BASELINE,
    });
    let mut layout: Layout<ColorBrush> = builder.build(text);
    layout.break_all_lines(None);

    env.check_layout_snapshot(&layout);
}

/// Test that a line height small enough to make the half-leading exceed the descent keeps the
/// resulting negative extent below the baseline, rather than clamping it to zero.
#[test]
fn lines_zero_line_height_keeps_negative_under_extent() {
    let font_size = 16.0;
    let line_height_px = 0.0;
    let text = "dig\npug\nyak";

    let mut env = TestEnv::new(test_name!(), None);
    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::FontSize(font_size));
    builder.push_default(LineHeight::Absolute(line_height_px));
    let mut layout: Layout<ColorBrush> = builder.build(text);
    layout.break_all_lines(None);

    let (ascent, descent) = roboto_ascent_descent(font_size);
    let half_leading = (line_height_px - (ascent.round() + descent.round())) / 2.;
    let expected_above = ascent.round() + half_leading.floor();
    let expected_below = line_height_px - expected_above;
    assert!(
        expected_below < 0.,
        "expected a negative extent below the baseline, but got {expected_below}"
    );

    for line in layout.lines() {
        let metrics = line.metrics();
        assert_eq!(
            metrics.baseline - metrics.block_min_coord,
            expected_above,
            "expected {expected_above} above the baseline"
        );
        assert_eq!(
            metrics.block_max_coord - metrics.baseline,
            expected_below,
            "expected {expected_below} below the baseline"
        );
        assert_eq!(
            metrics.block_max_coord - metrics.block_min_coord,
            line_height_px,
            "expected the line box height to equal the line height {line_height_px}"
        );
    }
    assert_eq!(layout.height(), line_height_px * 3.);
}

/// Test fractional line height with a positive leading.
///
/// The line box height must be increased by the leading,
/// divided correctly between the sides of the baseline.
fn lines_fractional_line_height_positive_leading_internal(
    test_name: &str,
    font_size: f32,
    line_height_px: f32,
) {
    // Expected outputs
    let (ascent, descent, line_box_height) = ascent_descent_box_height(font_size, line_height_px);

    // Compute
    let (mut env, layout, selections) = compute(test_name, font_size, line_height_px);

    // Confirm metrics
    assert_common_truths(&layout, ascent, descent, line_box_height, line_height_px);
    for line in layout.lines() {
        let metrics = line.metrics();
        let text_metrics = line_text_metrics(&line);
        assert!(
            text_metrics.leading > 0.,
            "expected positive leading, but got {}",
            text_metrics.leading
        );

        let above = metrics.baseline - metrics.block_min_coord;
        let below = metrics.block_max_coord - metrics.baseline;
        let above_leading = above - ascent.round();
        let below_leading = below - descent.round();
        assert!(
            above_leading < below_leading,
            "expected above leading {above_leading} to be less than below leading {below_leading}"
        );
    }

    // Verify visuals
    env.render_and_check_snapshot(&layout, None, &selections);
}

/// Test fractional line height with a positive leading.
///
/// The line box height must be increased by the leading,
/// divided correctly between the sides of the baseline.
#[test]
fn lines_fractional_line_height_positive_leading() {
    // Inputs
    let font_size = 16.0;
    let line_height_px = 24.1;

    // Run the test
    lines_fractional_line_height_positive_leading_internal(test_name!(), font_size, line_height_px);
}

/// Test fractional line height with a big positive leading.
///
/// The line box height must be increased by the leading,
/// divided correctly between the sides of the baseline.
#[test]
fn lines_fractional_line_height_big_positive_leading() {
    // Inputs
    let font_size = 16.0;
    let line_height_px = 46.66;

    lines_fractional_line_height_positive_leading_internal(test_name!(), font_size, line_height_px);
}

#[test]
fn lines_line_height_metrics_relative() {
    let mut env = TestEnv::new(test_name!(), None);

    let mut builder = env.ranged_builder(TEXT);
    builder.push_default(LineHeight::MetricsRelative(1.1));

    let mut layout = builder.build(TEXT);

    layout.break_all_lines(None);
    layout.align(Alignment::Start, AlignmentOptions::default());
    env.check_layout_snapshot(&layout);
}

#[test]
fn lines_line_height_size_relative() {
    let mut env = TestEnv::new(test_name!(), None);

    let mut builder = env.ranged_builder(TEXT);
    builder.push_default(LineHeight::FontSizeRelative(1.2));

    let mut layout = builder.build(TEXT);

    layout.break_all_lines(None);
    layout.align(Alignment::Start, AlignmentOptions::default());
    env.check_layout_snapshot(&layout);
}

#[test]
fn lines_line_height_absolute() {
    let mut env = TestEnv::new(test_name!(), None);

    let mut builder = env.ranged_builder(TEXT);
    builder.push_default(LineHeight::Absolute(20.0));

    let mut layout = builder.build(TEXT);

    layout.break_all_lines(None);
    layout.align(Alignment::Start, AlignmentOptions::default());
    env.check_layout_snapshot(&layout);
}

/// Test that line height changes within a run affects the lines they occur on.
#[test]
fn line_height_changes_per_line() {
    let text = "cat ray bat jay";
    let small = 10.;

    let mut env = TestEnv::new(test_name!(), None);
    let mut builder = env.ranged_builder(text);
    builder.push_default(LineHeight::Absolute(small));
    // The "c" of "cat".
    builder.push(LineHeight::MetricsRelative(1.5), 0..1);

    // The "a" of "jay".
    builder.push(LineHeight::MetricsRelative(2.5), 13..14);

    // Narrow enough for one word per line.
    let mut layout: Layout<ColorBrush> = builder.build(text);
    layout.break_all_lines(Some(1.0));

    env.render_and_check_snapshot(&layout, None, &[]);

    let mut lines = layout.lines();
    assert!(lines.next().unwrap().metrics().line_height > small);
    assert_eq!(lines.next().unwrap().metrics().line_height, small);
    assert_eq!(lines.next().unwrap().metrics().line_height, small);
    assert!(lines.next().unwrap().metrics().line_height > small);
    assert!(lines.next().is_none());
}

#[test]
fn line_height_change_inside_ligature() {
    let text = "ffi ffi";
    let small = 20.0;
    let large = 40.0;

    let mut env = TestEnv::new(test_name!(), None);
    let mut builder = env.ranged_builder(text);
    builder.push_default(LineHeight::Absolute(small));

    // The ligature forms, and gets the bigger line height from the middle "f"
    builder.push(LineHeight::Absolute(large), 5..6);

    // Narrow enough for one word per line.
    let mut layout: Layout<ColorBrush> = builder.build(text);
    layout.break_all_lines(Some(1.0));

    env.render_and_check_snapshot(&layout, None, &[]);

    let mut lines = layout.lines();
    assert_eq!(lines.next().unwrap().metrics().line_height, small);
    assert_eq!(lines.next().unwrap().metrics().line_height, large);
}

/// Metrics contributed by content that is moved to the next line when a word does not fit must
/// not leak into the line it was reverted from.
#[test]
fn lines_revert_restores_line_height() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "aaa BBB";
    let mut builder = env.ranged_builder(text);
    builder.push(StyleProperty::FontSize(64.0), 4..7);
    let mut layout = builder.build(text);

    // "aaa B" fits, so the large glyphs are added to the first line before the line breaker
    // reverts to the break opportunity after the space.
    layout.break_all_lines(Some(95.0));
    layout.align(Alignment::Start, AlignmentOptions::default());

    assert_eq!(layout.len(), 2);
    let heights: Vec<f32> = layout.lines().map(|l| l.metrics().line_height).collect();
    assert_eq!(heights, [16.0, 64.0]);
}

/// Like [`lines_revert_restores_line_height`], but the reverted content is in a
/// `vertical-align: top` span, i.e. not in the root aligned subtree.
#[test]
fn lines_revert_restores_aligned_subtree_line_height() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "aaa BBB";
    let mut builder = env.ranged_builder(text);
    builder.push(StyleProperty::FontSize(64.0), 4..7);
    builder.push(StyleProperty::VerticalAlign(VerticalAlign::TOP), 4..7);
    let mut layout = builder.build(text);

    layout.break_all_lines(Some(95.0));
    layout.align(Alignment::Start, AlignmentOptions::default());

    assert_eq!(layout.len(), 2);
    let heights: Vec<f32> = layout.lines().map(|l| l.metrics().line_height).collect();
    assert_eq!(heights, [16.0, 64.0]);
}

/// A `top` and a `bottom` aligned subtree spanning several words each, whose extents grow word
/// by word across line-breaking opportunities and reverts, on lines with both of them.
#[test]
fn lines_aligned_subtrees_grow_across_breaks() {
    let mut env = TestEnv::new(test_name!(), None);

    let mut builder = env.tree_builder();
    for (i, align) in [
        VerticalAlign::TOP,
        VerticalAlign::BOTTOM,
        VerticalAlign::TOP,
        VerticalAlign::BOTTOM,
    ]
    .into_iter()
    .enumerate()
    {
        builder.push_style_modification_span(&[StyleProperty::VerticalAlign(align)]);
        for (j, word) in ["aa ", "bbb ", "cc ", "ddd ", "ee "]
            .into_iter()
            .enumerate()
        {
            let size = 12.0 + 4.0 * ((i + j) % 4) as f32;
            // `vertical-align` is inherited, so reset it to keep the word in the subtree of the
            // enclosing `top`/`bottom` span.
            builder.push_style_modification_span(&[
                StyleProperty::FontSize(size),
                StyleProperty::VerticalAlign(VerticalAlign::BASELINE),
            ]);
            builder.push_text(word);
            builder.pop_style_span();
        }
        builder.pop_style_span();
    }
    let (mut layout, _) = builder.build();
    layout.break_all_lines(Some(130.0));
    layout.align(Alignment::Start, AlignmentOptions::default());

    let metrics: Vec<(f32, f32)> = layout
        .lines()
        .map(|l| (l.metrics().line_height, l.metrics().baseline))
        .collect();
    assert_eq!(
        metrics,
        [
            (24.0, 13.0),
            (24.0, 45.0),
            (24.0, 61.0),
            (24.0, 89.0),
            (24.0, 117.0)
        ]
    );
    env.check_layout_snapshot(&layout);
}
